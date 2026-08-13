//! The rich-string token intermediate representation.
//!
//! A decoded sheet string is text with embedded control tokens framed
//! `0x02 <code> <length> <payload> 0x03`. This module models it losslessly:
//! text runs keep their characters, tokens keep their code and their raw
//! payload bytes, and [`RichString::encode`] reproduces the input byte for
//! byte. The 26 codes present in the frozen client are named, and their
//! payloads use the recursive expression grammar modeled below. Unknown
//! codes and non-expression payloads remain lossless through their raw bytes.
//! Evidence: `docs/formats/ssd-sheet.md`, "Rich-string control tokens".

use crate::error::{ErrorKind, FormatError, Result};
use crate::reader::{to_hex, Span};

/// Byte that opens a control token.
pub const TOKEN_START: u8 = 0x02;

/// Byte that closes a control token.
pub const TOKEN_END: u8 = 0x03;

/// Lowest length byte that is an escape rather than a value.
pub const LENGTH_ESCAPE_FLOOR: u8 = 0xF0;

/// A control code present in the frozen 1.23b sheet corpus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MacroCode {
    SetTime,
    If,
    Switch,
    NewLine,
    Wait,
    Icon,
    Color,
    EdgeColor,
    SoftHyphen,
    Bold,
    Italic,
    NonBreakingSpace,
    Hyphen,
    Number,
    Kilo,
    Seconds,
    Time,
    Sheet,
    String,
    Head,
    Split,
    HeadAll,
    Lower,
    EnglishNoun,
    GermanNoun,
    FrenchNoun,
    Unknown(u8),
}

impl MacroCode {
    pub const fn from_byte(code: u8) -> Self {
        match code {
            0x07 => Self::SetTime,
            0x08 => Self::If,
            0x09 => Self::Switch,
            0x10 => Self::NewLine,
            0x11 => Self::Wait,
            0x12 => Self::Icon,
            0x13 => Self::Color,
            0x14 => Self::EdgeColor,
            0x16 => Self::SoftHyphen,
            0x19 => Self::Bold,
            0x1A => Self::Italic,
            0x1D => Self::NonBreakingSpace,
            0x1F => Self::Hyphen,
            0x20 => Self::Number,
            0x22 => Self::Kilo,
            0x24 => Self::Seconds,
            0x25 => Self::Time,
            0x28 => Self::Sheet,
            0x29 => Self::String,
            0x2B => Self::Head,
            0x2C => Self::Split,
            0x2D => Self::HeadAll,
            0x2F => Self::Lower,
            0x31 => Self::EnglishNoun,
            0x32 => Self::GermanNoun,
            0x33 => Self::FrenchNoun,
            other => Self::Unknown(other),
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::SetTime => "set-time",
            Self::If => "if",
            Self::Switch => "switch",
            Self::NewLine => "newline",
            Self::Wait => "wait",
            Self::Icon => "icon",
            Self::Color => "color",
            Self::EdgeColor => "edge-color",
            Self::SoftHyphen => "soft-hyphen",
            Self::Bold => "bold",
            Self::Italic => "italic",
            Self::NonBreakingSpace => "non-breaking-space",
            Self::Hyphen => "hyphen",
            Self::Number => "number",
            Self::Kilo => "kilo",
            Self::Seconds => "seconds",
            Self::Time => "time",
            Self::Sheet => "sheet",
            Self::String => "string",
            Self::Head => "head",
            Self::Split => "split",
            Self::HeadAll => "head-all",
            Self::Lower => "lower",
            Self::EnglishNoun => "english-noun",
            Self::GermanNoun => "german-noun",
            Self::FrenchNoun => "french-noun",
            Self::Unknown(_) => "unknown",
        }
    }
}

/// One expression in a control-token payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expression {
    Integer {
        value: u32,
        raw: Vec<u8>,
    },
    Placeholder(u8),
    Unary {
        code: u8,
        operand: Box<Expression>,
    },
    Binary {
        code: u8,
        left: Box<Expression>,
        right: Box<Expression>,
    },
    String {
        raw_length: Vec<u8>,
        value: RichString,
    },
}

/// How a token's payload length was written.
///
/// A byte below [`LENGTH_ESCAPE_FLOOR`] carries the length directly, offset
/// by one. Three escapes appear across the install and no other. A lead
/// byte outside this set is an error rather than a guess.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LengthEncoding {
    /// `n + 1`, one byte. 222777 of the install's 223513 tokens.
    Direct,
    /// `0xF0` then the length in one byte. 166 tokens.
    Byte,
    /// `0xF1` then one byte scaled by 256. One token in the whole install.
    ByteScaled,
    /// `0xF2` then the length in two big-endian bytes. 569 tokens.
    Word,
}

impl LengthEncoding {
    pub fn name(self) -> &'static str {
        match self {
            LengthEncoding::Direct => "direct",
            LengthEncoding::Byte => "byte",
            LengthEncoding::ByteScaled => "byte-scaled",
            LengthEncoding::Word => "word",
        }
    }
}

/// One control token, kept whole.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub span: Span,
    pub code: u8,
    pub encoding: LengthEncoding,
    /// The length bytes verbatim, so re-encoding never has to reconstruct
    /// a form the client wrote a different way.
    pub length_bytes: Vec<u8>,
    /// The payload verbatim, including any nested `0x02 .. 0x03` framing,
    /// which this crate does not descend into.
    pub payload: Vec<u8>,
}

/// A run of text or a token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Segment {
    Text { span: Span, text: String },
    Token(Token),
}

impl Segment {
    pub fn kind_name(&self) -> &'static str {
        match self {
            Segment::Text { .. } => "text",
            Segment::Token(_) => "token",
        }
    }

    pub fn span(&self) -> Span {
        match self {
            Segment::Text { span, .. } => *span,
            Segment::Token(token) => token.span,
        }
    }
}

/// A decoded string as text runs and tokens.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RichString {
    pub segments: Vec<Segment>,
}

impl RichString {
    /// Parse decoded string bytes. `base` is the absolute offset of
    /// `data[0]` in the file the bytes came from, so segment spans point at
    /// the *decoded* text. The encoded bytes they came from are the
    /// enclosing sheet string's span.
    pub fn parse(data: &[u8], base: u64) -> Result<Self> {
        let mut segments = Vec::new();
        let mut position = 0usize;
        let mut run_start = 0usize;

        while position < data.len() {
            if data[position] != TOKEN_START {
                position += 1;
                continue;
            }
            push_text(&mut segments, data, base, run_start, position)?;
            let token = parse_token(data, base, position)?;
            position = (token.span.end() - base) as usize;
            run_start = position;
            segments.push(Segment::Token(token));
        }
        push_text(&mut segments, data, base, run_start, data.len())?;
        Ok(Self { segments })
    }

    /// Rebuild the exact bytes this string was parsed from.
    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        for segment in &self.segments {
            match segment {
                Segment::Text { text, .. } => bytes.extend_from_slice(text.as_bytes()),
                Segment::Token(token) => {
                    bytes.push(TOKEN_START);
                    bytes.push(token.code);
                    bytes.extend_from_slice(&token.length_bytes);
                    bytes.extend_from_slice(&token.payload);
                    bytes.push(TOKEN_END);
                }
            }
        }
        bytes
    }

    /// The text with every token removed. This is a report, not the string:
    /// dropping the tokens loses information, which is why the IR is what
    /// the library returns.
    pub fn text_only(&self) -> String {
        let mut text = String::new();
        for segment in &self.segments {
            if let Segment::Text { text: run, .. } = segment {
                text.push_str(run);
            }
        }
        text
    }

    pub fn tokens(&self) -> impl Iterator<Item = &Token> {
        self.segments.iter().filter_map(|segment| match segment {
            Segment::Token(token) => Some(token),
            Segment::Text { .. } => None,
        })
    }

    /// A reversible CSV-cell view. Literal backslashes and opening square
    /// brackets are escaped. Each token carries its name and exact bytes.
    pub fn to_lossless_text(&self) -> String {
        let mut output = String::new();
        for segment in &self.segments {
            match segment {
                Segment::Text { text, .. } => {
                    for character in text.chars() {
                        if matches!(character, '\\' | '[') {
                            output.push('\\');
                        }
                        output.push(character);
                    }
                }
                Segment::Token(token) => {
                    output.push_str("[@");
                    output.push_str(token.macro_code().name());
                    output.push(':');
                    output.push_str(&to_hex(&token.raw_bytes()));
                    output.push(']');
                }
            }
        }
        output
    }
}

impl Token {
    pub const fn macro_code(&self) -> MacroCode {
        MacroCode::from_byte(self.code)
    }

    pub fn raw_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(self.span.length as usize);
        bytes.push(TOKEN_START);
        bytes.push(self.code);
        bytes.extend_from_slice(&self.length_bytes);
        bytes.extend_from_slice(&self.payload);
        bytes.push(TOKEN_END);
        bytes
    }

    /// Decode every expression in the payload. A payload that is not an
    /// expression stream returns the offset where decoding stopped.
    pub fn expressions(&self) -> std::result::Result<Vec<Expression>, usize> {
        let mut expressions = Vec::new();
        let mut position = 0usize;
        while position < self.payload.len() {
            let (expression, consumed) =
                parse_expression(&self.payload[position..]).ok_or(position)?;
            expressions.push(expression);
            position += consumed;
        }
        Ok(expressions)
    }
}

fn parse_expression(data: &[u8]) -> Option<(Expression, usize)> {
    let lead = *data.first()?;
    if (1..0xD0).contains(&lead) || (0xF0..=0xFE).contains(&lead) {
        let (value, consumed) = decode_integer(data)?;
        return Some((
            Expression::Integer {
                value,
                raw: data[..consumed].to_vec(),
            },
            consumed,
        ));
    }
    if (0xD0..=0xDF).contains(&lead) || lead == 0xEC {
        return Some((Expression::Placeholder(lead), 1));
    }
    if (0xE8..=0xEB).contains(&lead) {
        let (operand, consumed) = parse_expression(data.get(1..)?)?;
        return Some((
            Expression::Unary {
                code: lead,
                operand: Box::new(operand),
            },
            consumed + 1,
        ));
    }
    if (0xE0..=0xE5).contains(&lead) {
        let (left, left_length) = parse_expression(data.get(1..)?)?;
        let (right, right_length) = parse_expression(data.get(1 + left_length..)?)?;
        return Some((
            Expression::Binary {
                code: lead,
                left: Box::new(left),
                right: Box::new(right),
            },
            1 + left_length + right_length,
        ));
    }
    if lead == 0xFF {
        let tail = data.get(1..)?;
        let (length, length_size) = decode_integer(tail)?;
        let length = usize::try_from(length).ok()?;
        let begin = 1 + length_size;
        let end = begin.checked_add(length)?;
        let body = data.get(begin..end)?;
        return Some((
            Expression::String {
                raw_length: tail[..length_size].to_vec(),
                value: RichString::parse(body, 0).ok()?,
            },
            end,
        ));
    }
    None
}

fn decode_integer(data: &[u8]) -> Option<(u32, usize)> {
    let lead = *data.first()?;
    if (1..0xD0).contains(&lead) {
        return Some((u32::from(lead - 1), 1));
    }
    if !(0xF0..=0xFE).contains(&lead) {
        return None;
    }
    let mask = lead.wrapping_add(1) & 0x0F;
    let mut value = 0u32;
    let mut position = 1usize;
    for (bit, shift) in [(8, 24), (4, 16), (2, 8), (1, 0)] {
        if mask & bit == 0 {
            continue;
        }
        let byte = *data.get(position)?;
        if byte == 0 {
            return None;
        }
        value |= u32::from(byte) << shift;
        position += 1;
    }
    Some((value, position))
}

fn push_text(
    segments: &mut Vec<Segment>,
    data: &[u8],
    base: u64,
    start: usize,
    end: usize,
) -> Result<()> {
    if start >= end {
        return Ok(());
    }
    let bytes = &data[start..end];
    let text = std::str::from_utf8(bytes).map_err(|error| {
        FormatError::new(
            ErrorKind::InvalidUtf8,
            base + (start + error.valid_up_to()) as u64,
            "text outside the control tokens is not valid UTF-8",
        )
    })?;
    segments.push(Segment::Text {
        span: Span::new(base + start as u64, (end - start) as u64),
        text: text.to_string(),
    });
    Ok(())
}

fn parse_token(data: &[u8], base: u64, start: usize) -> Result<Token> {
    let bad = |offset: usize, detail: &str| {
        FormatError::new(
            ErrorKind::MalformedRichStringToken,
            base + offset as u64,
            detail.to_string(),
        )
    };

    let code = *data
        .get(start + 1)
        .ok_or_else(|| bad(start + 1, "a control token ends before its code"))?;
    let lead = *data
        .get(start + 2)
        .ok_or_else(|| bad(start + 2, "a control token ends before its length"))?;

    let (encoding, width) = match lead {
        0x00..=0xEF => (LengthEncoding::Direct, 0usize),
        0xF0 => (LengthEncoding::Byte, 1),
        0xF1 => (LengthEncoding::ByteScaled, 1),
        0xF2 => (LengthEncoding::Word, 2),
        other => {
            return Err(FormatError::new(
                ErrorKind::MalformedRichStringToken,
                base + (start + 2) as u64,
                format!(
                    "control token length lead byte 0x{other:02x} is not an encoding this crate has established"
                ),
            ))
        }
    };

    let length_start = start + 2;
    let length_end = length_start + 1 + width;
    let length_bytes = data
        .get(length_start..length_end)
        .ok_or_else(|| bad(length_start, "a control token ends inside its length"))?
        .to_vec();

    let payload_length = match encoding {
        LengthEncoding::Direct => {
            if lead == 0 {
                // The direct form carries n + 1, so zero cannot occur; the
                // client never writes it and this crate will not invent a
                // reading for it.
                return Err(bad(length_start, "control token length byte is zero"));
            }
            usize::from(lead - 1)
        }
        LengthEncoding::Byte => usize::from(length_bytes[1]),
        LengthEncoding::ByteScaled => usize::from(length_bytes[1]) << 8,
        LengthEncoding::Word => usize::from(u16::from_be_bytes([length_bytes[1], length_bytes[2]])),
    };

    let payload_end = length_end + payload_length;
    let payload = data
        .get(length_end..payload_end)
        .ok_or_else(|| bad(length_end, "a control token payload runs past the string"))?
        .to_vec();
    match data.get(payload_end) {
        Some(&TOKEN_END) => {}
        Some(other) => {
            return Err(FormatError::new(
                ErrorKind::MalformedRichStringToken,
                base + payload_end as u64,
                format!("a control token ends with 0x{other:02x}, not 0x03"),
            ))
        }
        None => return Err(bad(payload_end, "a control token does not close")),
    }

    Ok(Token {
        span: Span::new(base + start as u64, (payload_end + 1 - start) as u64),
        code,
        encoding,
        length_bytes,
        payload,
    })
}

/// Lowercase hex of a token payload, for reports that name a payload
/// without carrying client text.
pub fn payload_hex(payload: &[u8]) -> String {
    to_hex(payload)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(bytes: &[u8]) -> RichString {
        RichString::parse(bytes, 0).unwrap()
    }

    #[test]
    fn the_retail_macro_vocabulary_is_named() {
        let expected = [
            (0x07, "set-time"),
            (0x08, "if"),
            (0x09, "switch"),
            (0x10, "newline"),
            (0x11, "wait"),
            (0x12, "icon"),
            (0x13, "color"),
            (0x14, "edge-color"),
            (0x16, "soft-hyphen"),
            (0x19, "bold"),
            (0x1A, "italic"),
            (0x1D, "non-breaking-space"),
            (0x1F, "hyphen"),
            (0x20, "number"),
            (0x22, "kilo"),
            (0x24, "seconds"),
            (0x25, "time"),
            (0x28, "sheet"),
            (0x29, "string"),
            (0x2B, "head"),
            (0x2C, "split"),
            (0x2D, "head-all"),
            (0x2F, "lower"),
            (0x31, "english-noun"),
            (0x32, "german-noun"),
            (0x33, "french-noun"),
        ];
        assert_eq!(expected.len(), 26);
        for (code, name) in expected {
            assert_eq!(MacroCode::from_byte(code).name(), name);
        }
        assert_eq!(MacroCode::from_byte(0xAA), MacroCode::Unknown(0xAA));
    }

    #[test]
    fn expressions_cover_integers_parameters_comparisons_and_strings() {
        let token = Token {
            span: Span::new(0, 0),
            code: 0x08,
            encoding: LengthEncoding::Direct,
            length_bytes: vec![1],
            payload: vec![
                0xE4, 0xE8, 0x02, 0x03, 0xFF, 0x06, b'h', b'e', b'l', b'l', b'o',
            ],
        };
        let expressions = token.expressions().unwrap();
        assert_eq!(expressions.len(), 2);
        assert!(matches!(
            expressions[0],
            Expression::Binary { code: 0xE4, .. }
        ));
        match &expressions[1] {
            Expression::String { value, .. } => assert_eq!(value.text_only(), "hello"),
            other => panic!("string expression parsed as {other:?}"),
        }
    }

    #[test]
    fn csv_text_escapes_literals_and_keeps_exact_token_bytes() {
        let value = parse(b"literal\\[x]\x02\x10\x01\x03");
        assert_eq!(
            value.to_lossless_text(),
            "literal\\\\\\[x][@newline:02100103]"
        );
    }

    #[test]
    fn splits_text_and_tokens_and_round_trips() {
        // "ab" <token 0x10, direct length 0> "cd" <token 0x13, 5 bytes>
        let bytes = b"ab\x02\x10\x01\x03cd\x02\x13\x06\xfe\xff\xf3\xf3\xf3\x03";
        let rich = parse(bytes);
        assert_eq!(rich.segments.len(), 4);
        assert_eq!(rich.text_only(), "abcd");
        let codes: Vec<u8> = rich.tokens().map(|token| token.code).collect();
        assert_eq!(codes, [0x10, 0x13]);
        let payloads: Vec<String> = rich
            .tokens()
            .map(|token| payload_hex(&token.payload))
            .collect();
        assert_eq!(payloads, ["", "fefff3f3f3"]);
        assert_eq!(rich.encode(), bytes);
    }

    #[test]
    fn every_length_encoding_round_trips() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"\x02\x08\x03ab\x03");
        bytes.extend_from_slice(b"\x02\x09\xf0\x02cd\x03");
        bytes.extend_from_slice(b"\x02\x0a\xf2\x00\x03efg\x03");
        let rich = parse(&bytes);
        let encodings: Vec<LengthEncoding> = rich.tokens().map(|token| token.encoding).collect();
        assert_eq!(
            encodings,
            [
                LengthEncoding::Direct,
                LengthEncoding::Byte,
                LengthEncoding::Word
            ]
        );
        assert_eq!(rich.encode(), bytes);

        // The scaled form: 0xF1 then one byte, times 256.
        let mut scaled = vec![TOKEN_START, 0x08, 0xF1, 0x01];
        scaled.extend(std::iter::repeat(0x41u8).take(256));
        scaled.push(TOKEN_END);
        let rich = parse(&scaled);
        assert_eq!(rich.tokens().count(), 1);
        assert_eq!(rich.tokens().next().unwrap().payload.len(), 256);
        assert_eq!(rich.encode(), scaled);
    }

    #[test]
    fn a_nested_frame_stays_inside_the_payload() {
        // The payload holds its own 0x02 .. 0x03 pair; this crate keeps it
        // whole rather than descending into it.
        let bytes = b"\x02\x28\x05\x02\x08\x01\x03\x03";
        let rich = parse(bytes);
        assert_eq!(rich.segments.len(), 1);
        let token = rich.tokens().next().unwrap();
        assert_eq!(token.code, 0x28);
        assert_eq!(payload_hex(&token.payload), "02080103");
        assert_eq!(rich.encode(), bytes);
    }

    #[test]
    fn malformed_tokens_carry_their_offset() {
        for (bytes, offset) in [
            (b"ab\x02\x10\x09xx".to_vec(), 5usize),
            (b"ab\x02\x10\x03xx\x04".to_vec(), 7),
            (b"ab\x02".to_vec(), 3),
            (b"ab\x02\x10".to_vec(), 4),
            (b"ab\x02\x10\x00\x03".to_vec(), 4),
            (b"ab\x02\x10\xf3\x00\x03".to_vec(), 4),
        ] {
            let error = RichString::parse(&bytes, 0).unwrap_err();
            assert_eq!(
                error.kind(),
                ErrorKind::MalformedRichStringToken,
                "{bytes:02x?}: {error}"
            );
            assert_eq!(error.offset(), offset as u64, "{bytes:02x?}");
        }
    }

    #[test]
    fn text_that_is_not_utf8_fails_at_the_bad_byte() {
        let error = RichString::parse(b"ok\xC3\x28", 100).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidUtf8);
        assert_eq!(error.offset(), 102);
    }

    #[test]
    fn multibyte_text_survives_a_round_trip() {
        let bytes = "\u{30A8}\u{30E9}".as_bytes();
        let rich = parse(bytes);
        assert_eq!(rich.text_only(), "\u{30A8}\u{30E9}");
        assert_eq!(rich.encode(), bytes);
    }
}
