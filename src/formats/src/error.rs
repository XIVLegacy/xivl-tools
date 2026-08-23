//! Typed parse failures.
//!
//! Every failure names a stable kind and the absolute byte offset in the
//! original input where the parser stopped. Conformance cases assert the
//! kind string, so the strings are contract surface: renaming one is a
//! breaking change to every case that names it.

use std::fmt;

/// Stable identifier for a parse failure.
///
/// The kebab-case rendering is what a conformance case puts in
/// `expect.errorKind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorKind {
    /// The input ended before a field the parser had already committed to.
    UnexpectedEndOfInput,
    /// The leading bytes are not a container signature this crate reads.
    BadMagic,
    /// A declared header size cannot hold the fields the format defines.
    HeaderTooSmall,
    /// A declared header size runs past the end of the input.
    HeaderSizeOutOfRange,
    /// The advisory size field at 0x10 runs past the end of the input.
    DeclaredSizeOutOfRange,
    /// A RES container's two subresource counts disagree.
    SubresourceCountMismatch,
    /// A subresource directory cannot fit inside the container.
    SubresourceCountOutOfRange,
    /// Nested containers are deeper than the parser will follow.
    NestingTooDeep,
    /// A resource identifier is not eight hexadecimal digits.
    InvalidResourceId,
    /// A DAT path does not match the resource-path convention.
    InvalidResourcePath,
    /// Bytes that must be text are not valid UTF-8.
    InvalidUtf8,
    /// A Lua resource path contains a byte outside the ASCII path domain.
    InvalidLuaPath,
    /// An LPB wrapper does not decode to a Lua 5.1 chunk signature.
    InvalidLuaChunk,
    /// A Lua chunk header names a representation outside the evidenced target.
    UnsupportedLuaHeader,
    /// A Lua bytecode table, tag, string, or terminator is malformed.
    MalformedLuaBytecode,
    /// A declared Lua bytecode structure exceeds a parser resource limit.
    ResourceLimitExceeded,
    /// A complete Lua chunk leaves bytes outside its root prototype.
    TrailingBytes,
    /// An XML document does not parse: an unclosed tag, a mismatched end
    /// tag, an unquoted attribute, or trailing content after the root.
    MalformedXml,
    /// An XML construct outside the subset these documents use. The reader
    /// refuses it rather than guessing at a meaning the client never
    /// exercises. See `docs/formats/ssd-sheet.md`.
    UnsupportedXmlConstruct,
    /// An element sits where the document grammar does not allow it.
    UnexpectedElement,
    /// An attribute or element text is not the value its name requires.
    InvalidAttributeValue,
    /// A fixed-width record array ends in a partial record.
    TrailingPartialRecord,
    /// A sheet string does not end in the terminator its marker implies.
    MalformedSheetString,
    /// A rich-string control token does not close, or declares a length in
    /// an encoding this crate has not established.
    MalformedRichStringToken,
    /// A sheet column names a type whose width this crate has not
    /// established against retail data.
    UnknownColumnType,
    /// An input read as a scrambled document does not end in the container
    /// trailer byte, so it is not one.
    MissingScrambleTrailer,
    /// A container whose key is its own file name was handed bytes without
    /// a name. Nothing in the bytes supplies one.
    MissingContainerName,
}

impl ErrorKind {
    /// The stable string form used by conformance cases and CLI output.
    pub const fn as_str(self) -> &'static str {
        match self {
            ErrorKind::UnexpectedEndOfInput => "unexpected-end-of-input",
            ErrorKind::BadMagic => "bad-magic",
            ErrorKind::HeaderTooSmall => "header-too-small",
            ErrorKind::HeaderSizeOutOfRange => "header-size-out-of-range",
            ErrorKind::DeclaredSizeOutOfRange => "declared-size-out-of-range",
            ErrorKind::SubresourceCountMismatch => "subresource-count-mismatch",
            ErrorKind::SubresourceCountOutOfRange => "subresource-count-out-of-range",
            ErrorKind::NestingTooDeep => "nesting-too-deep",
            ErrorKind::InvalidResourceId => "invalid-resource-id",
            ErrorKind::InvalidResourcePath => "invalid-resource-path",
            ErrorKind::InvalidUtf8 => "invalid-utf8",
            ErrorKind::InvalidLuaPath => "invalid-lua-path",
            ErrorKind::InvalidLuaChunk => "invalid-lua-chunk",
            ErrorKind::UnsupportedLuaHeader => "unsupported-lua-header",
            ErrorKind::MalformedLuaBytecode => "malformed-lua-bytecode",
            ErrorKind::ResourceLimitExceeded => "resource-limit-exceeded",
            ErrorKind::TrailingBytes => "trailing-bytes",
            ErrorKind::MalformedXml => "malformed-xml",
            ErrorKind::UnsupportedXmlConstruct => "unsupported-xml-construct",
            ErrorKind::UnexpectedElement => "unexpected-element",
            ErrorKind::InvalidAttributeValue => "invalid-attribute-value",
            ErrorKind::TrailingPartialRecord => "trailing-partial-record",
            ErrorKind::MalformedSheetString => "malformed-sheet-string",
            ErrorKind::MalformedRichStringToken => "malformed-rich-string-token",
            ErrorKind::UnknownColumnType => "unknown-column-type",
            ErrorKind::MissingScrambleTrailer => "missing-scramble-trailer",
            ErrorKind::MissingContainerName => "missing-container-name",
        }
    }
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// A parse failure: what went wrong, and where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatError {
    kind: ErrorKind,
    offset: u64,
    detail: String,
}

impl FormatError {
    pub fn new(kind: ErrorKind, offset: u64, detail: impl Into<String>) -> Self {
        Self {
            kind,
            offset,
            detail: detail.into(),
        }
    }

    pub fn kind(&self) -> ErrorKind {
        self.kind
    }

    /// Absolute offset from the start of the input the parser was given.
    pub fn offset(&self) -> u64 {
        self.offset
    }

    pub fn detail(&self) -> &str {
        &self.detail
    }
}

impl fmt::Display for FormatError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} at offset {}: {}",
            self.kind, self.offset, self.detail
        )
    }
}

impl std::error::Error for FormatError {}

pub type Result<T> = std::result::Result<T, FormatError>;
